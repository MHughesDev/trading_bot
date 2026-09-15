"""AT-61 — the eval task that measures whether a resume is bit-identical.

`checkpoint.BIT_IDENTICAL_FRAMEWORKS` is a **record of measurements**. This
module is the measurement. Nothing else may add a name to that set.

## What it does, per framework

1. Train `n` steps in a fresh process. Record the parameters, the predictions
   and the metrics.
2. Train `n − 2` steps in a second fresh process, write a checkpoint, and let
   the parent **kill the process** — `Popen.kill()`, not a clean shutdown, so
   nothing survives except what reached the disk.
3. Resume from that checkpoint in a third fresh process, train the remaining
   two steps, and record the same three things.
4. Compare 1 and 3.

Three processes rather than three function calls is the point. An in-process
"resume" can pass while silently reading a module-level generator, a cached
dataset or a warm allocator that a restarted worker would not have. The kill is
real for the same reason: a checkpoint that is only correct because the writing
process was still alive is not a checkpoint.

## What "bit-identical" is allowed to mean here

The easy way to make this test pass is to compare something weaker than the
weights — a metric rounded to four decimals, a loss curve "close enough", a
serialization that prints six significant digits. A test that passes by being
lenient is worse than no test, because it licenses a claim nobody checked. So:

* **Weights** are the framework's own parameters as raw IEEE-754 bytes, compared
  with `==` on `bytes`. No tolerance, no rounding, no `allclose`.
* **Metrics** never become decimal strings. They are carried as
  `struct.pack("<d", value)` from the moment they are computed, so there is no
  point at which a difference could be rounded away.
* **Predictions** over a fixed matrix are compared as `float64` bytes as well.
  This is the check that cannot be fooled by a serialization detail: if any
  parameter really differs, the predictions differ.

All three must match. `_mutation_probe` exists so the suite can prove the
comparator is not vacuous — flip one bit of one parameter and every framework's
comparison must fail.

## Outcomes

`bit_identical`      — every configuration measured reproduced exactly.
`diverged`           — at least one configuration did not, with the cause named.
`no_checkpoint_boundary` — the framework's fit has no step to resume from, so
                       there is no checkpoint to be bit-identical about.
`not_installed`      — not measured here. **Not** a pass.
"""

from __future__ import annotations

import argparse
import base64
import importlib.util
import io
import json
import os
import pathlib
import random
import struct
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from typing import Any, Callable

import numpy as np

# ───────────────────────────────────────────────────────────────────────────────
# The experiment
# ───────────────────────────────────────────────────────────────────────────────

#: `n` in AT-61. Small on purpose: the test must run on every PR.
STEPS = 6
#: AT-61 says resume from the checkpoint at `n − 2`.
CHECKPOINT_STEP = STEPS - 2

SEED = 20260915
N_ROWS = 256
N_FEATURES = 6
N_VALIDATION = 64
BATCH_SIZE = 64

#: How long the parent waits for the child to write its checkpoint before giving
#: up. Generous — a cold `import torch` on a loaded CI box is not fast.
CHECKPOINT_TIMEOUT_S = 300.0

BIT_IDENTICAL = "bit_identical"
DIVERGED = "diverged"
NO_CHECKPOINT_BOUNDARY = "no_checkpoint_boundary"
NOT_INSTALLED = "not_installed"


def dataset() -> tuple[np.ndarray, np.ndarray]:
    """The same rows in every process. Generated, not loaded, so the eval has no
    fixture to drift from."""
    rng = np.random.default_rng(SEED)
    x = rng.normal(size=(N_ROWS, N_FEATURES))
    coef = np.arange(1, N_FEATURES + 1, dtype=float)
    y = x @ coef + rng.normal(size=N_ROWS) * 0.25
    return x, y


def _split(x: np.ndarray, y: np.ndarray):
    cut = N_ROWS - N_VALIDATION
    return x[:cut], y[:cut], x[cut:], y[cut:]


# ───────────────────────────────────────────────────────────────────────────────
# Outputs and their comparison
# ───────────────────────────────────────────────────────────────────────────────


def pack(value: float) -> bytes:
    """A metric as its IEEE-754 bytes.

    Metrics are packed at the moment they are computed and never unpacked for
    comparison, so "equal to four decimals" is not a thing this module can
    accidentally assert. It also makes NaN compare equal to an identical NaN,
    which is the answer you want: two runs that both diverged to NaN in the same
    place did the same thing.
    """
    return struct.pack("<d", float(value))


@dataclass(frozen=True)
class Outputs:
    """What a run produced. All three fields are raw bytes."""

    weights: bytes
    predictions: bytes
    #: metric name -> packed double. Ordered by key on the wire.
    metrics: dict[str, bytes]

    def to_json(self) -> str:
        return json.dumps(
            {
                "weights": base64.b64encode(self.weights).decode("ascii"),
                "predictions": base64.b64encode(self.predictions).decode("ascii"),
                "metrics": {
                    k: base64.b64encode(v).decode("ascii")
                    for k, v in sorted(self.metrics.items())
                },
            }
        )

    @staticmethod
    def from_json(text: str) -> "Outputs":
        raw = json.loads(text)
        return Outputs(
            weights=base64.b64decode(raw["weights"]),
            predictions=base64.b64decode(raw["predictions"]),
            metrics={k: base64.b64decode(v) for k, v in raw["metrics"].items()},
        )


@dataclass(frozen=True)
class Comparison:
    weights_equal: bool
    predictions_equal: bool
    metrics_equal: bool
    #: The first thing found to differ, for the report. Never used to decide.
    detail: str | None = None

    @property
    def identical(self) -> bool:
        return self.weights_equal and self.predictions_equal and self.metrics_equal


def compare(a: Outputs, b: Outputs) -> Comparison:
    """Byte-wise, on all three. No tolerance anywhere in this function."""
    weights_equal = a.weights == b.weights
    predictions_equal = a.predictions == b.predictions
    metrics_equal = a.metrics == b.metrics

    detail: str | None = None
    if not weights_equal:
        detail = _first_byte_difference("weights", a.weights, b.weights)
    elif not predictions_equal:
        detail = _first_byte_difference("predictions", a.predictions, b.predictions)
    elif not metrics_equal:
        names = sorted(set(a.metrics) | set(b.metrics))
        differing = [n for n in names if a.metrics.get(n) != b.metrics.get(n)]
        detail = f"metrics differ: {', '.join(differing)}"
    return Comparison(weights_equal, predictions_equal, metrics_equal, detail)


def _first_byte_difference(what: str, a: bytes, b: bytes) -> str:
    if len(a) != len(b):
        return f"{what} differ in length: {len(a)} vs {len(b)} bytes"
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            return f"{what} differ from byte {i} of {len(a)}"
    return f"{what} differ"


def _mutation_probe(outputs: Outputs) -> Outputs:
    """The same outputs with the low bit of the last weight byte flipped.

    Used by the suite to prove `compare` is capable of failing. A comparator
    that cannot detect a one-bit change is the failure mode this whole module
    exists to prevent.
    """
    if not outputs.weights:
        raise ValueError("nothing to mutate: the run recorded no weights")
    mutated = bytearray(outputs.weights)
    mutated[-1] ^= 0x01
    return Outputs(bytes(mutated), outputs.predictions, dict(outputs.metrics))


# ───────────────────────────────────────────────────────────────────────────────
# torch
# ───────────────────────────────────────────────────────────────────────────────


def _torch_build(config: str, input_dim: int):
    """The architectures the platform's torch adapter can build, plus one with a
    stochastic layer.

    `mlp` and `lstm` are `torch_adapter._build_model`'s two arms. `mlp_dropout`
    is not something the adapter builds today, and it is measured anyway: a
    resume that is only bit-identical because nothing in the forward pass draws
    from the RNG has not demonstrated that the RNG is being restored. It is the
    configuration that would break first.
    """
    import torch
    from torch import nn

    hidden = 32
    if config == "lstm":

        class LSTMNet(nn.Module):
            def __init__(self) -> None:
                super().__init__()
                self.lstm = nn.LSTM(input_dim, hidden, batch_first=True)
                self.head = nn.Linear(hidden, 1)

            def forward(self, x):  # noqa: D102
                if x.dim() == 2:
                    x = x.unsqueeze(1)
                out, _ = self.lstm(x)
                return self.head(out[:, -1, :])

        return LSTMNet()

    layers: list[Any] = [nn.Linear(input_dim, hidden), nn.ReLU()]
    if config == "mlp_dropout":
        layers.append(nn.Dropout(0.25))
    layers.append(nn.Linear(hidden, 1))
    return nn.Sequential(*layers)


def _torch_weights(model) -> bytes:
    """Every parameter and buffer, as the raw bytes of its tensor.

    Sorted by name, with the name, dtype and shape in the stream, so a renamed
    or reshaped parameter is a difference rather than a coincidence.
    """
    out = io.BytesIO()
    for name, tensor in sorted(model.state_dict().items()):
        out.write(name.encode("utf-8"))
        out.write(str(tensor.dtype).encode("utf-8"))
        out.write(str(tuple(tensor.shape)).encode("utf-8"))
        out.write(tensor.detach().cpu().contiguous().numpy().tobytes())
    return out.getvalue()


def _torch_segment(
    config: str, state: bytes | None, from_step: int, to_step: int
) -> tuple[bytes, Outputs]:
    import torch
    from torch import nn

    torch.set_num_threads(1)
    torch.use_deterministic_algorithms(True)

    x, y = dataset()
    x_tr, y_tr, x_val, y_val = _split(x, y)
    xt = torch.tensor(x_tr, dtype=torch.float32)
    yt = torch.tensor(y_tr, dtype=torch.float32).unsqueeze(1)
    xv = torch.tensor(x_val, dtype=torch.float32)
    yv = torch.tensor(y_val, dtype=torch.float32).unsqueeze(1)

    # Construction consumes the default generator, in both the fresh process and
    # the resuming one, so that restoring the saved generator state lands on the
    # same stream position rather than merely a plausible one.
    random.seed(SEED)
    np.random.seed(SEED % (2**32))
    torch.manual_seed(SEED)
    model = _torch_build(config, x_tr.shape[1])
    opt = torch.optim.Adam(model.parameters(), lr=1e-3)
    # A schedule that actually moves inside `n` steps: without it the scheduler
    # state is constant and restoring it proves nothing.
    sched = torch.optim.lr_scheduler.StepLR(opt, step_size=2, gamma=0.5)
    scaler = torch.amp.GradScaler("cpu", enabled=False)
    sampler = torch.Generator()
    sampler.manual_seed(SEED + 1)
    loss_fn = nn.MSELoss()

    n = len(xt)
    perm = torch.randperm(n, generator=sampler)
    position = 0
    step = 0

    if state is not None:
        # `weights_only=False`: this blob is one we wrote, one directory over,
        # and it carries the optimizer, the schedule and three RNG states — none
        # of which is a tensor.
        blob = torch.load(io.BytesIO(state), weights_only=False)
        if blob["step"] != from_step:
            raise ValueError(
                f"checkpoint is at step {blob['step']}, asked to resume from {from_step}"
            )
        model.load_state_dict(blob["model"])
        opt.load_state_dict(blob["optimizer"])
        sched.load_state_dict(blob["scheduler"])
        scaler.load_state_dict(blob["scaler"])
        sampler.set_state(blob["sampler"])
        torch.set_rng_state(blob["torch_cpu_rng"])
        np.random.set_state(blob["numpy_rng"])
        random.setstate(blob["python_rng"])
        perm = blob["permutation"]
        position = blob["dataloader_position"]
        step = blob["step"]

    train_loss = float("nan")
    while step < to_step:
        if position >= n:
            perm = torch.randperm(n, generator=sampler)
            position = 0
        idx = perm[position : position + BATCH_SIZE]
        position += len(idx)

        model.train()
        opt.zero_grad(set_to_none=True)
        loss = loss_fn(model(xt[idx]), yt[idx])
        loss.backward()
        opt.step()
        sched.step()
        step += 1
        train_loss = float(loss.item())

    new_state = io.BytesIO()
    torch.save(
        {
            "step": step,
            "model": model.state_dict(),
            "optimizer": opt.state_dict(),
            "scheduler": sched.state_dict(),
            "scaler": scaler.state_dict(),
            "sampler": sampler.get_state(),
            "torch_cpu_rng": torch.get_rng_state(),
            "numpy_rng": np.random.get_state(),
            "python_rng": random.getstate(),
            "permutation": perm,
            "dataloader_position": position,
        },
        new_state,
    )

    model.eval()
    with torch.no_grad():
        val_pred = model(xv).squeeze(-1).to(torch.float64).numpy()
        val_loss = float(loss_fn(model(xv), yv).item())
    metrics = {
        "final_train_loss": pack(train_loss),
        "final_val_loss": pack(val_loss),
        "learning_rate": pack(opt.param_groups[0]["lr"]),
    }
    return new_state.getvalue(), Outputs(
        _torch_weights(model), val_pred.tobytes(), metrics
    )


# ───────────────────────────────────────────────────────────────────────────────
# xgboost
# ───────────────────────────────────────────────────────────────────────────────

#: `deterministic` mirrors `xgboost_adapter.train`'s own defaults — `subsample`
#: and `colsample_bytree` both 1.0. The other two are configurations the
#: platform's hyperparameter space permits and an HPO sweep will reach.
XGB_CONFIGS = {
    "deterministic": {},
    "row_subsample": {"subsample": 0.7},
    "column_subsample": {"colsample_bytree": 0.6},
}


def _xgb_params(config: str) -> dict[str, Any]:
    params: dict[str, Any] = {
        "max_depth": 4,
        "eta": 0.1,
        "objective": "reg:squarederror",
        "tree_method": "hist",
        "seed": SEED % (2**31),
        # One thread: a multi-threaded histogram reduction is a *different*
        # question from resume, and mixing the two would leave the cause of any
        # difference ambiguous.
        "nthread": 1,
        # Pinned rather than estimated from the labels, so the resumed booster
        # cannot inherit a different intercept.
        "base_score": 0.5,
    }
    params.update(XGB_CONFIGS[config])
    return params


def _xgb_segment(
    config: str, state: bytes | None, from_step: int, to_step: int
) -> tuple[bytes, Outputs]:
    import xgboost as xgb

    x, y = dataset()
    x_tr, y_tr, x_val, y_val = _split(x, y)
    dtrain = xgb.DMatrix(x_tr, label=y_tr)
    dval = xgb.DMatrix(x_val, label=y_val)

    previous = None
    if state is not None:
        blob = json.loads(state)
        if blob["round"] != from_step:
            raise ValueError(
                f"checkpoint is at round {blob['round']}, asked to resume from {from_step}"
            )
        previous = xgb.Booster(model_file=bytearray(base64.b64decode(blob["raw"])))

    booster = xgb.train(
        _xgb_params(config),
        dtrain,
        num_boost_round=to_step - from_step,
        xgb_model=previous,
    )

    # UBJSON, not JSON: the binary form carries the doubles themselves rather
    # than a decimal rendering of them.
    raw = bytes(booster.save_raw("ubj"))
    new_state = json.dumps(
        {"round": to_step, "raw": base64.b64encode(raw).decode("ascii")}
    ).encode("utf-8")

    val_pred = booster.predict(dval).astype(np.float64)
    train_pred = booster.predict(dtrain).astype(np.float64)
    metrics = {
        "train_mse": pack(float(np.mean((train_pred - y_tr) ** 2))),
        "val_mse": pack(float(np.mean((val_pred - y_val) ** 2))),
    }
    return new_state, Outputs(raw, val_pred.tobytes(), metrics)


# ───────────────────────────────────────────────────────────────────────────────
# lightgbm
# ───────────────────────────────────────────────────────────────────────────────

LGB_CONFIGS = {
    "deterministic": {},
    "bagging": {"bagging_fraction": 0.7, "bagging_freq": 1},
    "feature_fraction": {"feature_fraction": 0.6},
}

#: The one line of a LightGBM model string that is about the *call* rather than
#: the model: it echoes the `num_boost_round` argument of the last `lgb.train`.
#: An uninterrupted six-round run writes `[num_iterations: 6]`; a run that
#: resumed for its last two writes `[num_iterations: 2]` with byte-identical
#: trees. Excluding it is the only leniency in this module, it is exactly one
#: line, and `test_the_lightgbm_exclusion_hides_nothing` proves that excluding it
#: still catches a changed leaf value.
LGB_CALL_ARGUMENT_LINE = "[num_iterations: "


def _lgb_params(config: str) -> dict[str, Any]:
    params: dict[str, Any] = {
        "objective": "regression",
        "learning_rate": 0.1,
        "num_leaves": 7,
        "min_data_in_leaf": 5,
        "num_threads": 1,
        "seed": SEED % (2**31),
        # `checkpoint.DETERMINISM_FLAGS["lightgbm"]`, which the manifest records.
        # Without `force_row_wise` LightGBM builds histograms in thread-
        # scheduling order and differs run to run on one machine, which would
        # make any resume result unreadable.
        "deterministic": True,
        "force_row_wise": True,
        "verbose": -1,
    }
    params.update(LGB_CONFIGS[config])
    return params


def lgb_model_weights(model_string: str) -> bytes:
    """A LightGBM model string with the call-argument line removed.

    # Raises
    `ValueError` if the line is absent or appears more than once — if LightGBM's
    format changes, this must fail loudly rather than start excluding something
    else.
    """
    lines = model_string.splitlines()
    hits = [i for i, line in enumerate(lines) if line.startswith(LGB_CALL_ARGUMENT_LINE)]
    if len(hits) != 1:
        raise ValueError(
            f"expected exactly one `{LGB_CALL_ARGUMENT_LINE}...]` line in the LightGBM "
            f"model string, found {len(hits)}"
        )
    del lines[hits[0]]
    return "\n".join(lines).encode("utf-8")


def _lgb_segment(
    config: str, state: bytes | None, from_step: int, to_step: int
) -> tuple[bytes, Outputs]:
    import lightgbm as lgb

    x, y = dataset()
    x_tr, y_tr, x_val, y_val = _split(x, y)

    previous = None
    if state is not None:
        blob = json.loads(state)
        if blob["round"] != from_step:
            raise ValueError(
                f"checkpoint is at round {blob['round']}, asked to resume from {from_step}"
            )
        previous = lgb.Booster(model_str=blob["model"])

    booster = lgb.train(
        _lgb_params(config),
        lgb.Dataset(x_tr, label=y_tr, free_raw_data=False),
        num_boost_round=to_step - from_step,
        init_model=previous,
    )

    model_string = booster.model_to_string()
    new_state = json.dumps({"round": to_step, "model": model_string}).encode("utf-8")

    val_pred = booster.predict(x_val).astype(np.float64)
    train_pred = booster.predict(x_tr).astype(np.float64)
    metrics = {
        "train_mse": pack(float(np.mean((train_pred - y_tr) ** 2))),
        "val_mse": pack(float(np.mean((val_pred - y_val) ** 2))),
    }
    return new_state, Outputs(
        lgb_model_weights(model_string), val_pred.tobytes(), metrics
    )


# ───────────────────────────────────────────────────────────────────────────────
# The framework table
# ───────────────────────────────────────────────────────────────────────────────

Segment = Callable[[str, bytes | None, int, int], tuple[bytes, Outputs]]


@dataclass(frozen=True)
class Framework:
    name: str
    #: The import that has to exist for this framework to be measurable at all.
    module: str | None
    configs: tuple[str, ...]
    segment: Segment | None
    #: Set when the framework has no step boundary to checkpoint at. The eval
    #: then reports `no_checkpoint_boundary` rather than running anything, and
    #: `test_a_framework_without_a_step_boundary_says_so` proves the claim
    #: against the adapter's source rather than taking it on trust.
    no_boundary_cause: str | None = None


FRAMEWORKS: tuple[Framework, ...] = (
    Framework("torch", "torch", ("mlp", "lstm", "mlp_dropout"), _torch_segment),
    Framework("xgboost", "xgboost", tuple(XGB_CONFIGS), _xgb_segment),
    Framework("lightgbm", "lightgbm", tuple(LGB_CONFIGS), _lgb_segment),
    Framework(
        "sklearn",
        "sklearn",
        (),
        None,
        no_boundary_cause=(
            "the platform's sklearn adapter fits every estimator in a single "
            "`fit` call (RandomForest/GradientBoosting/LogisticRegression/Ridge) "
            "and sets no `warm_start`, so there is no step n-2 at which a "
            "checkpoint could be written"
        ),
    ),
    Framework(
        "garch",
        "arch",
        (),
        None,
        no_boundary_cause=(
            "the GARCH adapter is one maximum-likelihood fit through `arch`, "
            "which exposes no iterate state, so there is no step to resume from"
        ),
    ),
)

BY_NAME = {f.name: f for f in FRAMEWORKS}


def is_installed(framework: Framework) -> bool:
    if framework.module is None:
        return True
    return importlib.util.find_spec(framework.module) is not None


# ───────────────────────────────────────────────────────────────────────────────
# The driver: three processes, one of them killed
# ───────────────────────────────────────────────────────────────────────────────

_TRAINER_ROOT = pathlib.Path(__file__).resolve().parents[1]


def _child(args: list[str], *, capture: pathlib.Path):
    """Spawn one worker. Returns the process and the stderr handle the caller
    must close — an open handle is what makes a Windows temp-directory cleanup
    fail, which would look like a test failure and be a plumbing bug."""
    env = dict(os.environ)
    env["PYTHONHASHSEED"] = "0"
    handle = capture.open("wb")
    proc = subprocess.Popen(  # noqa: S603 - fixed argv, our own module
        [sys.executable, "-m", "app.resume_eval", *args],
        cwd=str(_TRAINER_ROOT),
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=handle,
    )
    return proc, handle


def _run_to_completion(args: list[str], work: pathlib.Path, tag: str) -> None:
    err = work / f"{tag}.stderr"
    proc, handle = _child(args, capture=err)
    try:
        code = proc.wait(timeout=CHECKPOINT_TIMEOUT_S)
    finally:
        handle.close()
    if code != 0:
        raise RuntimeError(
            f"the {tag} process exited {code}:\n{err.read_text(errors='replace')}"
        )


@dataclass(frozen=True)
class RunPair:
    uninterrupted: Outputs
    resumed: Outputs
    #: The exit status of the killed process. Recorded so the test can assert the
    #: process really was killed rather than allowed to finish.
    killed_exit_code: int | None


def run_pair(framework: str, config: str, work: pathlib.Path) -> RunPair:
    """Train `n`; train `n−2`, kill, resume; return both results."""
    uninterrupted_out = work / "uninterrupted.json"
    _run_to_completion(
        [
            "--mode", "uninterrupted",
            "--framework", framework,
            "--config", config,
            "--to-step", str(STEPS),
            "--out", str(uninterrupted_out),
        ],
        work,
        "uninterrupted",
    )

    state = work / "checkpoint.state"
    ready = work / "checkpoint.ready"
    err = work / "checkpoint.stderr"
    proc, handle = _child(
        [
            "--mode", "checkpoint",
            "--framework", framework,
            "--config", config,
            "--to-step", str(CHECKPOINT_STEP),
            "--state", str(state),
            "--ready", str(ready),
        ],
        capture=err,
    )
    deadline = time.monotonic() + CHECKPOINT_TIMEOUT_S
    try:
        while not ready.exists():
            if proc.poll() is not None:
                raise RuntimeError(
                    "the checkpointing process exited before writing a checkpoint:\n"
                    + err.read_text(errors="replace")
                )
            if time.monotonic() > deadline:
                raise TimeoutError(f"no checkpoint after {CHECKPOINT_TIMEOUT_S}s")
            time.sleep(0.05)
        # The kill. Not a signal it could handle, not a clean shutdown: from here
        # on the only thing that exists is the file on disk.
        proc.kill()
    finally:
        killed_exit_code = proc.wait(timeout=60)
        handle.close()

    resumed_out = work / "resumed.json"
    _run_to_completion(
        [
            "--mode", "resume",
            "--framework", framework,
            "--config", config,
            "--from-step", str(CHECKPOINT_STEP),
            "--to-step", str(STEPS),
            "--state", str(state),
            "--out", str(resumed_out),
        ],
        work,
        "resume",
    )

    return RunPair(
        Outputs.from_json(uninterrupted_out.read_text()),
        Outputs.from_json(resumed_out.read_text()),
        killed_exit_code,
    )


# ───────────────────────────────────────────────────────────────────────────────
# Evidence
# ───────────────────────────────────────────────────────────────────────────────


def run_twice_uninterrupted(framework: str, config: str, work: pathlib.Path) -> Comparison:
    """Two uninterrupted runs of the same configuration, compared.

    This is what separates "the resume lost something" from "this configuration
    is not deterministic in the first place". Without it, a divergence has two
    possible causes and the report would have to guess at which.
    """
    outs = []
    for i in (0, 1):
        target = work / f"repeat-{i}.json"
        _run_to_completion(
            [
                "--mode", "uninterrupted",
                "--framework", framework,
                "--config", config,
                "--to-step", str(STEPS),
                "--out", str(target),
            ],
            work,
            f"repeat-{i}",
        )
        outs.append(Outputs.from_json(target.read_text()))
    return compare(outs[0], outs[1])


@dataclass(frozen=True)
class ConfigEvidence:
    framework: str
    config: str
    identical: bool
    comparison: Comparison | None
    killed_exit_code: int | None = None
    #: What the uninterrupted run produced. Kept so the suite can take real
    #: weights, flip one bit and prove the comparator notices.
    uninterrupted: Outputs | None = None
    #: Two uninterrupted runs compared, measured only where the resume diverged.
    #: `identical` here means the configuration is deterministic, so the resume
    #: is the thing that lost the information.
    repeatability: Comparison | None = None

    @property
    def cause(self) -> str | None:
        if self.identical:
            return None
        return self.comparison.detail if self.comparison else "not measured"


@dataclass(frozen=True)
class FrameworkEvidence:
    framework: str
    outcome: str
    configs: tuple[ConfigEvidence, ...] = ()
    cause: str | None = None

    @property
    def deterministic_config_identical(self) -> bool:
        """Whether the configuration with no stochastic sampling reproduced.

        This is what separates "cannot resume" from "cannot resume while
        sampling", and the two deserve different answers.
        """
        for c in self.configs:
            if c.config in ("deterministic", "mlp"):
                return c.identical
        return False

    @property
    def stochastic_configs_that_diverged(self) -> tuple[str, ...]:
        return tuple(
            c.config
            for c in self.configs
            if not c.identical and c.config not in ("deterministic", "mlp")
        )


def measure(framework_name: str, *, work_root: pathlib.Path | None = None) -> FrameworkEvidence:
    """Run AT-61 for one framework, across every configuration it declares."""
    framework = BY_NAME[framework_name]

    if framework.no_boundary_cause is not None:
        return FrameworkEvidence(
            framework_name, NO_CHECKPOINT_BOUNDARY, (), framework.no_boundary_cause
        )
    if not is_installed(framework):
        return FrameworkEvidence(
            framework_name,
            NOT_INSTALLED,
            (),
            f"`{framework.module}` is not importable here, so nothing was measured; "
            f"an unmeasured framework is not a passing one",
        )

    results: list[ConfigEvidence] = []
    with tempfile.TemporaryDirectory(prefix=f"at61-{framework_name}-") as tmp:
        for config in framework.configs:
            work = pathlib.Path(tmp) / config
            work.mkdir(parents=True)
            pair = run_pair(framework_name, config, work)
            comparison = compare(pair.uninterrupted, pair.resumed)
            # Only where it diverged: the question "is this configuration even
            # deterministic?" is only interesting once the answer to the first
            # question is no, and two more processes per configuration is a real
            # cost on a per-PR test.
            repeat = (
                None if comparison.identical
                else run_twice_uninterrupted(framework_name, config, work)
            )
            results.append(
                ConfigEvidence(
                    framework_name,
                    config,
                    comparison.identical,
                    comparison,
                    pair.killed_exit_code,
                    pair.uninterrupted,
                    repeat,
                )
            )

    configs = tuple(results)
    if all(c.identical for c in configs):
        return FrameworkEvidence(framework_name, BIT_IDENTICAL, configs)

    failed = [c for c in configs if not c.identical]
    cause = "; ".join(f"{c.config}: {c.cause}" for c in failed)
    return FrameworkEvidence(framework_name, DIVERGED, configs, cause)


def measure_all(names: tuple[str, ...] | None = None) -> tuple[FrameworkEvidence, ...]:
    return tuple(measure(n) for n in (names or tuple(BY_NAME)))


# ───────────────────────────────────────────────────────────────────────────────
# The worker entry point
# ───────────────────────────────────────────────────────────────────────────────


def _segment_for(framework_name: str) -> Segment:
    segment = BY_NAME[framework_name].segment
    if segment is None:
        raise ValueError(f"{framework_name} has no resumable training segment")
    return segment


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--mode",
        choices=("uninterrupted", "checkpoint", "resume", "report"),
        default="report",
    )
    parser.add_argument("--framework")
    parser.add_argument("--config")
    parser.add_argument("--from-step", type=int, default=0)
    parser.add_argument("--to-step", type=int, default=STEPS)
    parser.add_argument("--state")
    parser.add_argument("--ready")
    parser.add_argument("--out")
    args = parser.parse_args(argv)

    if args.mode == "report":
        for evidence in measure_all():
            print(f"{evidence.framework:<10} {evidence.outcome}")
            for config in evidence.configs:
                mark = "ok  " if config.identical else "DIFF"
                print(f"    {mark} {config.config}: {config.cause or 'bit-identical'}")
            if evidence.cause:
                print(f"    cause: {evidence.cause}")
        return 0

    segment = _segment_for(args.framework)

    if args.mode == "uninterrupted":
        _, outputs = segment(args.config, None, 0, args.to_step)
        pathlib.Path(args.out).write_text(outputs.to_json())
        return 0

    if args.mode == "checkpoint":
        state, _ = segment(args.config, None, 0, args.to_step)
        target = pathlib.Path(args.state)
        staging = target.with_suffix(".partial")
        staging.write_bytes(state)
        os.replace(staging, target)
        pathlib.Path(args.ready).write_text("ready")
        # Wait to be killed. The parent is what ends this process; anything this
        # process could do to end itself cleanly would weaken the test.
        while True:
            time.sleep(0.05)

    if args.mode == "resume":
        state = pathlib.Path(args.state).read_bytes()
        _, outputs = segment(args.config, state, args.from_step, args.to_step)
        pathlib.Path(args.out).write_text(outputs.to_json())
        return 0

    return 1


if __name__ == "__main__":
    raise SystemExit(main())
