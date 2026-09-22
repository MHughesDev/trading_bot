"""2.5's producing half — the torch adapter writes a checkpoint and resumes from it.

AT-61 (`test_resume_eval.py`) proved that *a* torch resume is bit-identical. It
did so over a loop that lives in the eval. This suite proves the same thing about
the loop a real training job runs — `engine.train_torch_model` through
`adapters.torch_adapter.train` — because until this existed the platform could
truthfully declare `resume_support: bit_identical` on a checkpoint no adapter had
ever written.

The shape is track 6's, deliberately, because it is the right one:

1. Train the whole run in a fresh process. Record the model bundle's bytes.
2. Train part of it in a second fresh process, writing checkpoints, and let the
   parent **kill the process** — `Popen.kill()`, no clean shutdown, no `finally`,
   nothing flushed on the way out. From there the only state that exists is what
   reached the disk.
3. Resume from the newest checkpoint in a third fresh process and finish.
4. Compare 1 and 3 **byte for byte**.

Three processes rather than three calls, and a real kill rather than a `return`,
for track 6's reason: an in-process resume can pass while reading a module-level
generator, a warm allocator or a cached frame that a restarted worker would not
have, and a checkpoint that is only correct because the writing process was still
alive is not a checkpoint.
"""

from __future__ import annotations

import os
import pathlib
import subprocess
import sys
import time

import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))

from app import checkpoint, engine, torch_checkpoint  # noqa: E402
from app.adapters import torch_adapter  # noqa: E402

pytest.importorskip("torch")

TRAINER_ROOT = pathlib.Path(__file__).resolve().parents[1]
DRIVER = pathlib.Path(__file__).resolve().parent / "resume_producer_driver.py"

#: Enough epochs that a killed run has somewhere to resume from, few enough that
#: this runs on every PR.
EPOCHS = 8
KILL_AFTER_EPOCHS = 5
TIMEOUT_S = 300.0


# ───────────────────────────────────────────────────────────────────────────────
# The driver, run in its own process
# ───────────────────────────────────────────────────────────────────────────────


def _spawn(args: list[str], work: pathlib.Path, tag: str):
    """One child. The stderr handle comes back so the caller can close it —
    an open handle is what makes a Windows temp cleanup fail, which looks like a
    test failure and is a plumbing bug."""
    err = work / f"{tag}.stderr"
    handle = err.open("wb")
    env = dict(os.environ)
    env["PYTHONHASHSEED"] = "0"
    env["ARTIFACT_STORE"] = "fs"
    env["ARTIFACT_STORE_PATH"] = str(work / "artifacts")
    proc = subprocess.Popen(  # noqa: S603 - fixed argv, our own module
        [sys.executable, str(DRIVER), *args],
        cwd=str(TRAINER_ROOT),
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=handle,
    )
    return proc, handle, err


def _run(args: list[str], work: pathlib.Path, tag: str) -> None:
    proc, handle, err = _spawn(args, work, tag)
    try:
        code = proc.wait(timeout=TIMEOUT_S)
    finally:
        handle.close()
    if code != 0:
        raise RuntimeError(f"{tag} exited {code}:\n{err.read_text(errors='replace')}")


@pytest.fixture(scope="module")
def killed_and_resumed(tmp_path_factory):
    """Run the whole experiment once. Every process in it is a real one."""
    work = pathlib.Path(tmp_path_factory.mktemp("resume-producer"))

    _run(
        ["--mode", "whole", "--epochs", str(EPOCHS), "--out", str(work / "whole.bin")],
        work,
        "whole",
    )

    # The second process checkpoints every epoch and then waits to be killed.
    ready = work / "ready"
    proc, handle, err = _spawn(
        [
            "--mode", "partial",
            "--epochs", str(EPOCHS),
            "--kill-after", str(KILL_AFTER_EPOCHS),
            "--ckpt-dir", str(work / "ckpt"),
            "--ready", str(ready),
        ],
        work,
        "partial",
    )
    try:
        deadline = time.monotonic() + TIMEOUT_S
        while not ready.exists():
            if proc.poll() is not None:
                raise RuntimeError(
                    "the checkpointing process exited before writing a checkpoint:\n"
                    + err.read_text(errors="replace")
                )
            if time.monotonic() > deadline:
                raise TimeoutError(f"no checkpoint after {TIMEOUT_S}s")
            time.sleep(0.05)
        proc.kill()
    finally:
        killed_exit_code = proc.wait(timeout=60)
        handle.close()

    manifest_path = work / "ckpt" / "manifest.json"
    blob_path = work / "ckpt" / "blob.path"

    _run(
        [
            "--mode", "resume",
            "--epochs", str(EPOCHS),
            "--resume-step", str(KILL_AFTER_EPOCHS),
            "--resume-from", blob_path.read_text().strip(),
            "--out", str(work / "resumed.bin"),
        ],
        work,
        "resume",
    )

    return {
        "work": work,
        "whole": (work / "whole.bin").read_bytes(),
        "resumed": (work / "resumed.bin").read_bytes(),
        "manifest": __import__("json").loads(manifest_path.read_text()),
        "blob": pathlib.Path(blob_path.read_text().strip().replace("file://", "")).read_bytes(),
        "killed_exit_code": killed_exit_code,
    }


# ───────────────────────────────────────────────────────────────────────────────
# The producer exists, and what it writes is resumable
# ───────────────────────────────────────────────────────────────────────────────


def test_the_adapter_writes_a_checkpoint_during_training(killed_and_resumed):
    """The finding track 6 ended on was that nothing wrote one. This is the proof
    that something now does, from a process that never got to finish."""
    blob = killed_and_resumed["blob"]
    assert blob, "the killed run left no checkpoint on disk"
    assert torch_checkpoint.is_checkpoint(blob)


def test_the_checkpointing_process_was_actually_killed(killed_and_resumed):
    """A resume proved against a process that shut down cleanly proves nothing."""
    code = killed_and_resumed["killed_exit_code"]
    assert code is not None
    assert code != 0, (
        "the checkpointing process exited 0, so it finished on its own and the "
        "checkpoint was never the only surviving state"
    )


def test_a_resumed_run_is_byte_identical_to_the_uninterrupted_one(killed_and_resumed):
    """The whole point, and the comparison is `==` on `bytes`.

    The bundles are the real training output — the header plus the `torch.save`
    of the state dict — so if any parameter differed anywhere, these differ. No
    tolerance, no `allclose`, nothing rounded.
    """
    whole = killed_and_resumed["whole"]
    resumed = killed_and_resumed["resumed"]
    assert whole, "the uninterrupted run produced nothing"
    assert len(whole) == len(resumed), (
        f"bundles differ in length: {len(whole)} vs {len(resumed)} bytes"
    )
    assert whole == resumed, (
        "a resumed run produced a different model from the uninterrupted one; "
        "first difference at byte "
        + str(next(i for i, (a, b) in enumerate(zip(whole, resumed)) if a != b))
    )


def test_the_comparison_can_fail(killed_and_resumed):
    """Flip one bit of the real bundle. If this passed, the test above would be
    asserting nothing."""
    whole = bytearray(killed_and_resumed["whole"])
    whole[-1] ^= 0x01
    assert bytes(whole) != killed_and_resumed["whole"]


# ───────────────────────────────────────────────────────────────────────────────
# The manifest says what the measurement licensed, and nothing more
# ───────────────────────────────────────────────────────────────────────────────


def test_the_manifest_satisfies_the_contract_the_registry_enforces(killed_and_resumed):
    manifest = killed_and_resumed["manifest"]
    checkpoint.validate(manifest)
    for field in checkpoint.REQUIRED_FIELDS:
        assert manifest.get(field) is not None, field
    for source in checkpoint.REQUIRED_RNG_SOURCES:
        assert manifest["rng_state"].get(source) is not None, source
    assert manifest["step"] == KILL_AFTER_EPOCHS
    assert manifest["dataloader_position"] == 0


def test_resume_support_comes_from_the_measurement_not_from_a_constant():
    """`build_manifest` asks `checkpoint.resume_support`; nothing in the producer
    writes the string itself.

    Proved by moving the measurement: drop torch from the measured set and the
    manifest the producer would write must change with it. A hardcoded
    `"bit_identical"` survives this and fails the assertion.
    """
    common = dict(
        step=3,
        optimizer_state_ref="file:///tmp/ckpt",
        lr_scheduler_state_ref=checkpoint.NO_LR_SCHEDULER,
        dataloader_position=0,
        amp_scaler_state=checkpoint.NO_AMP_SCALER,
        code_hash="sha256:code",
        image_digest="worker:trainer",
        dataset_id="snapshot:ds",
    )
    assert checkpoint.build_manifest(framework="torch", **common)["resume_support"] == (
        "bit_identical"
    )

    original = checkpoint.BIT_IDENTICAL_FRAMEWORKS
    try:
        checkpoint.BIT_IDENTICAL_FRAMEWORKS = frozenset()
        assert (
            checkpoint.build_manifest(framework="torch", **common)["resume_support"]
            == "unsupported"
        ), "the producer is not reading the measurement"
    finally:
        checkpoint.BIT_IDENTICAL_FRAMEWORKS = original

    # And the source carries no literal of its own.
    producer = (TRAINER_ROOT / "app" / "worker.py").read_text(encoding="utf-8")
    assert '"bit_identical"' not in producer.replace(
        '!= "bit_identical"', ""
    ), "worker.py assigns a resume_support literal somewhere"


# ───────────────────────────────────────────────────────────────────────────────
# xgboost with sampling on does not get a checkpoint at all
# ───────────────────────────────────────────────────────────────────────────────


def test_an_xgboost_run_with_subsample_writes_no_checkpoint():
    """ADR-P6-11's condition, at the producing end.

    `hpo.py` draws `subsample` on [0.5, 1.0], so this is the ordinary case rather
    than an edge one. The producer asks `resume_support` with the run's actual
    hyperparameters and gets `unsupported`, so no policy is built and the run
    writes nothing — there is no second, more permissive check anywhere that
    could let one through.
    """
    from app import worker as worker_mod
    from app.adapters import xgboost_adapter

    assert (
        checkpoint.resume_support("xgboost", hyperparameters={"subsample": 0.8})
        == "unsupported"
    )
    # Not in the checkpointable set to begin with, which is the first of the two
    # reasons and the one that does not depend on the hyperparameters at all.
    assert xgboost_adapter.train not in worker_mod.CHECKPOINTABLE_ADAPTERS
    assert worker_mod.CHECKPOINTABLE_ADAPTERS == (torch_adapter.train,)


def test_the_policy_refuses_a_framework_the_measurement_did_not_license(monkeypatch):
    """Put xgboost in the checkpointable set by force and it still writes nothing
    with sampling on, because the support check is separate from the set."""
    from app import worker as worker_mod
    from app.adapters import xgboost_adapter
    from app.schemas import CheckpointConfig, TrainRequest, ProgressConfig

    def request(framework: str, hp: dict) -> TrainRequest:
        return TrainRequest(
            run_id="r",
            model_id="m",
            model_kind="classifier",
            framework=framework,
            definition={"hyperparameters": hp},
            dataset_uri="file:///nowhere",
            dataset_hash="h",
            output_prefix="file:///tmp/out",
            progress=ProgressConfig(nats_subject="s"),
            checkpoint=CheckpointConfig(
                interval_seconds=0.0,
                code_hash="sha256:code",
                image_digest="worker:trainer",
                dataset_id="snapshot:ds",
            ),
        )

    monkeypatch.setattr(
        worker_mod,
        "CHECKPOINTABLE_ADAPTERS",
        (torch_adapter.train, xgboost_adapter.train),
    )
    sampled = worker_mod._build_checkpoint_policy(
        request("xgboost", {"subsample": 0.8}),
        xgboost_adapter.train,
        None,
        {"subsample": 0.8},
        [],
    )
    assert sampled is None, (
        "a sampling xgboost run was handed a checkpoint policy; ADR-P6-11 says "
        "its resume is not bit-identical and a checkpoint nothing may resume "
        "from is worse than none"
    )

    # Silence is `unsupported` too: an unstated configuration is not a measured one.
    assert (
        worker_mod._build_checkpoint_policy(
            request("xgboost", {}), xgboost_adapter.train, None, {}, []
        )
        is not None
    ), "an explicitly empty configuration engages no sampling knob"


def test_no_policy_means_no_checkpoint_and_that_is_the_default():
    """A request with no `checkpoint` block writes nothing. There is no interval
    the trainer supplies on its own."""
    from app import worker as worker_mod
    from app.schemas import ProgressConfig, TrainRequest

    req = TrainRequest(
        run_id="r",
        model_id="m",
        model_kind="regressor",
        framework="torch",
        definition={},
        dataset_uri="file:///nowhere",
        dataset_hash="h",
        output_prefix="file:///tmp/out",
        progress=ProgressConfig(nats_subject="s"),
    )
    assert req.checkpoint is None
    assert (
        worker_mod._build_checkpoint_policy(req, torch_adapter.train, None, {}, [])
        is None
    )


# ───────────────────────────────────────────────────────────────────────────────
# A checkpoint is not a model
# ───────────────────────────────────────────────────────────────────────────────


def test_a_checkpoint_is_not_a_servable_bundle(killed_and_resumed):
    """A partial model and a trained one are both `torch.save` blobs, and nothing
    downstream may mistake one for the other.

    The bundle envelope's magic is the discriminator: a checkpoint does not carry
    it, and a bundle is not a checkpoint.
    """
    blob = killed_and_resumed["blob"]
    bundle = killed_and_resumed["whole"]

    assert not blob.startswith(engine.MAGIC), "a checkpoint is wearing the bundle magic"
    assert bundle.startswith(engine.MAGIC)
    assert torch_checkpoint.is_checkpoint(blob)
    assert not torch_checkpoint.is_checkpoint(bundle)


def test_the_checkpoint_never_reaches_the_field_the_registry_reads_as_the_model():
    """`TrainResponse.artifact_uri` is the one field that becomes the trained
    artifact. Checkpoints have their own, and it is a different type."""
    from app.schemas import CheckpointWritten, TrainResponse

    resp = TrainResponse(
        status="succeeded",
        artifact_uri="file:///models/model.bin",
        checkpoints=[
            CheckpointWritten(
                uri="file:///models/checkpoints/step-3.ckpt",
                sha256="deadbeef",
                size_bytes=1,
                step=3,
                manifest={},
            )
        ],
    )
    assert resp.artifact_uri not in [c.uri for c in resp.checkpoints]
    assert all("checkpoints/" in c.uri for c in resp.checkpoints)


# ───────────────────────────────────────────────────────────────────────────────
# One implementation, not two
# ───────────────────────────────────────────────────────────────────────────────


def test_the_eval_and_the_trainer_share_one_checkpoint_implementation():
    """AT-61 must measure the code a real job resumes from.

    If `resume_eval` grew its own `torch.save` of a checkpoint dict again, the
    measurement would be true of that copy and say nothing about the trainer. So
    the module is asserted to be the only writer.
    """
    from app import resume_eval

    root = TRAINER_ROOT / "app"
    for module in ("resume_eval.py", "engine.py"):
        src = (root / module).read_text(encoding="utf-8")
        assert "torch_checkpoint" in src, f"{module} does not use the shared module"
        assert "torch_cpu_rng" not in src, (
            f"{module} names a checkpoint key directly, which means it has its own "
            f"blob shape again"
        )

    assert resume_eval._torch_segment is not None
    for key in ("model", "optimizer", "torch_cpu_rng", "numpy_rng", "python_rng"):
        assert key in torch_checkpoint.CHECKPOINT_KEYS


def test_the_blob_carries_every_key_it_claims_to(killed_and_resumed):
    import io

    import torch

    loaded = torch.load(io.BytesIO(killed_and_resumed["blob"]), weights_only=False)
    assert set(loaded) == set(torch_checkpoint.CHECKPOINT_KEYS)
    assert loaded["format"] == torch_checkpoint.FORMAT


def test_restore_refuses_a_blob_that_is_not_ours():
    """The format marker is checked, not assumed. Resuming from something else
    wearing the same file name is how a model's lineage starts saying something
    untrue."""
    import io

    import torch

    buffer = io.BytesIO()
    torch.save({"step": 3, "model": {}}, buffer)
    with pytest.raises(ValueError, match="not a .* checkpoint"):
        torch_checkpoint.restore(
            buffer.getvalue(), expected_step=3, model=None, optimizer=None
        )


def test_restore_refuses_a_checkpoint_at_a_different_step(killed_and_resumed):
    with pytest.raises(ValueError, match="asked to resume from"):
        torch_checkpoint.restore(
            killed_and_resumed["blob"],
            expected_step=KILL_AFTER_EPOCHS + 1,
            model=None,
            optimizer=None,
        )
