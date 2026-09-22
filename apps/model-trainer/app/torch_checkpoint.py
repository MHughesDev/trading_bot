"""The one torch checkpoint blob, and the only code that writes or reads it.

AT-61 (`app/resume_eval.py`) measured that a torch resume is bit-identical. It
measured *this* blob: `_torch_segment` calls `capture` and `restore` here, and so
does the training loop in `engine.train_torch_model` that the torch adapter runs.
One implementation with two callers rather than two implementations, because two
resume paths that disagree is a worse failure than one that is absent — the
measurement would be true of the eval's copy and say nothing about the trainer's.

## What is in the blob, and why each piece

Everything the next step consumes and cannot recompute:

* **model / optimizer** — Adam's moments are most of what the optimizer *is*.
* **scheduler / scaler** — a learning-rate schedule and an AMP loss scale are
  running state, not functions of the step number.
* **sampler / permutation / dataloader_position** — where the epoch had got to.
  Resume at the top of the epoch and the model sees some rows twice and some not
  at all.
* **the three RNGs** — python, numpy and torch's CPU generator. Leave one out and
  the resumed run draws a different dropout mask or a different shuffle, the loss
  curve still looks plausible, and nothing announces the divergence.

The optional pieces are `None` rather than absent when a loop does not have them:
a `scheduler` key holding `None` says "this run had no schedule", and a missing
`scheduler` key says "somebody's writer forgot". Only one of those is checkable,
which is the same reason `RngState.torch_cuda` is an empty list rather than an
omitted field.

## Not a model

`FORMAT` is the first thing written and the first thing read. A checkpoint is a
*partial* model: resuming from one is correct, serving from one is not, and the
two are both `torch.save` blobs. The marker is what lets
`engine.read_bundle_header` refuse a checkpoint by name instead of half-loading
it, and `test_resume_producer.py` holds it to that.
"""

from __future__ import annotations

import io
import random
from dataclasses import dataclass
from typing import Any

import numpy as np

#: Written into every blob and required on read. Bump it when the shape changes,
#: so an old checkpoint is refused rather than silently misread.
FORMAT = "tbot.torch.checkpoint.v1"

#: Every key a blob carries. Asserted by the test suite against a real blob, so a
#: writer that drops one fails there rather than in a resumed run six weeks later.
CHECKPOINT_KEYS: tuple[str, ...] = (
    "format",
    "step",
    "model",
    "optimizer",
    "scheduler",
    "scaler",
    "sampler",
    "torch_cpu_rng",
    "numpy_rng",
    "python_rng",
    "permutation",
    "dataloader_position",
)


@dataclass(frozen=True)
class Restored:
    """What the caller cannot get back by asking its own objects.

    The model, optimizer, scheduler, scaler and sampler are restored **in place**
    — `load_state_dict` on the objects the caller passed — so they are not
    returned. These three are plain values the loop owns.
    """

    step: int
    permutation: Any | None
    dataloader_position: int


def capture(
    *,
    step: int,
    model,
    optimizer,
    scheduler=None,
    scaler=None,
    sampler=None,
    permutation=None,
    dataloader_position: int = 0,
) -> bytes:
    """Everything needed to continue from `step`, as bytes.

    The caller is responsible for calling this at a point where `step` is a real
    boundary — no half-applied gradient, no partially consumed batch. An epoch
    boundary is one; so is a completed optimizer step.
    """
    import torch

    buffer = io.BytesIO()
    torch.save(
        {
            "format": FORMAT,
            "step": int(step),
            "model": model.state_dict(),
            "optimizer": optimizer.state_dict(),
            "scheduler": None if scheduler is None else scheduler.state_dict(),
            "scaler": None if scaler is None else scaler.state_dict(),
            "sampler": None if sampler is None else sampler.get_state(),
            "torch_cpu_rng": torch.get_rng_state(),
            "numpy_rng": np.random.get_state(),
            "python_rng": random.getstate(),
            "permutation": permutation,
            "dataloader_position": int(dataloader_position),
        },
        buffer,
    )
    return buffer.getvalue()


def restore(
    blob: bytes,
    *,
    expected_step: int,
    model,
    optimizer,
    scheduler=None,
    scaler=None,
    sampler=None,
) -> Restored:
    """Put a captured run back the way it was.

    `expected_step` is checked rather than trusted. A caller that resumes from a
    checkpoint at a different step than it believes produces a model whose step
    count is wrong and whose schedule is off by the difference, and neither shows
    up in any metric.

    # Raises
    `ValueError` if the blob is not one of ours, or is at a different step.
    """
    import torch

    # `weights_only=False`: this blob is one we wrote, and it deliberately carries
    # the optimizer, the schedule and three RNG states — none of which is a
    # tensor. The format marker below is the check that it is in fact ours.
    loaded = torch.load(io.BytesIO(blob), weights_only=False)

    if not isinstance(loaded, dict) or loaded.get("format") != FORMAT:
        raise ValueError(
            f"not a {FORMAT} checkpoint: refusing to resume from a blob whose "
            f"shape nobody has checked"
        )
    if loaded["step"] != expected_step:
        raise ValueError(
            f"checkpoint is at step {loaded['step']}, asked to resume from {expected_step}"
        )

    model.load_state_dict(loaded["model"])
    optimizer.load_state_dict(loaded["optimizer"])
    if scheduler is not None:
        scheduler.load_state_dict(loaded["scheduler"])
    if scaler is not None:
        scaler.load_state_dict(loaded["scaler"])
    if sampler is not None:
        sampler.set_state(loaded["sampler"])
    torch.set_rng_state(loaded["torch_cpu_rng"])
    np.random.set_state(loaded["numpy_rng"])
    random.setstate(loaded["python_rng"])

    return Restored(
        step=int(loaded["step"]),
        permutation=loaded["permutation"],
        dataloader_position=int(loaded["dataloader_position"]),
    )


def is_checkpoint(blob: bytes) -> bool:
    """Whether these bytes are a checkpoint rather than a servable model.

    Cheap and total: it reads the blob and looks for the marker, and answers
    `False` for anything it cannot read. Used by the bundle reader to refuse a
    checkpoint by name.
    """
    try:
        import torch

        loaded = torch.load(io.BytesIO(blob), weights_only=False)
    except Exception:  # noqa: BLE001 - anything unreadable is not our checkpoint
        return False
    return isinstance(loaded, dict) and loaded.get("format") == FORMAT
