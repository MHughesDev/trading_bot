"""The producing half of resumption (checklist 2.4/2.5; AT-61; OPEN-TASKS C1.2).

`test_resume_eval.py` proves torch resumes bit-identically from a *hand-built*
checkpoint. This proves the same thing through the path production uses: the
epoch loop in `engine.train_torch_model` writes the checkpoint itself, the run is
killed, and a fresh process restores what reached the store.
"""
from __future__ import annotations

import hashlib
import pathlib
import random
import sys

import numpy as np
import pytest

torch = pytest.importorskip("torch")

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from app import checkpoint, engine  # noqa: E402


class MemStore:
    """What `ArtifactStore` offers, in a dict."""

    def __init__(self):
        self.blobs: dict[str, bytes] = {}

    def put(self, key: str, data: bytes):
        uri = f"mem://{key}"
        self.blobs[uri] = data
        return uri, hashlib.sha256(data).hexdigest(), len(data)

    def get(self, uri: str) -> bytes:
        return self.blobs[uri]


def data():
    r = np.random.default_rng(7)
    x = r.normal(size=(96, 4)).astype(np.float32)
    y = (x[:, 0] * 2.0 - x[:, 1] + r.normal(0, 0.1, 96)).astype(np.float32)
    return x[:72], y[:72], x[72:], y[72:]


def seed_all(seed: int) -> None:
    random.seed(seed)
    np.random.seed(seed)
    torch.manual_seed(seed)


def build():
    return torch.nn.Sequential(
        torch.nn.Linear(4, 8), torch.nn.ReLU(), torch.nn.Linear(8, 1)
    )


def weights(model) -> bytes:
    return b"".join(
        t.detach().numpy().tobytes() for _, t in sorted(model.state_dict().items())
    )


def train(epochs, *, checkpoint_cfg=None, seed=11, emit=None):
    seed_all(seed)
    model = build()
    xt, yt, xv, yv = data()
    engine.train_torch_model(
        model, xt, yt, xv, yv,
        objective="regression", epochs=epochs, lr=1e-2, batch_size=16,
        emit_progress=emit or (lambda *a, **k: None),
        checkpoint=checkpoint_cfg,
    )
    return model


def cfg(store, *, resume=None, mtbf_s=1e-12):
    return {
        "key_prefix": "run/checkpoints",
        "code_hash": "sha256:code",
        "image_digest": "sha256:image",
        "dataset_id": "ds:abc",
        "mtbf_s": mtbf_s,
        "resume": resume,
    }, store


@pytest.fixture
def mem_store(monkeypatch):
    store = MemStore()
    from app import artifact_store

    monkeypatch.setattr(artifact_store, "_STORE", store)
    return store


class Killed(Exception):
    pass


def test_a_run_killed_after_a_checkpoint_resumes_to_the_uninterrupted_weights(mem_store):
    epochs = 6
    reference = weights(train(epochs))

    # The run that gets killed: a tiny MTBF makes the interval ~0, so it
    # checkpoints after every epoch, and the process dies on the epoch-4 frame
    # - after epoch 3's checkpoint was written and before epoch 4's.
    published: list[dict] = []

    def emit(phase, pct, metric=None):
        if phase == "checkpoint":
            published.append(metric["checkpoint"])
        if phase == "fitting" and metric and metric["epoch"] == 4:
            raise Killed()

    with pytest.raises(Killed):
        train(epochs, checkpoint_cfg=cfg(mem_store)[0], emit=emit)
    assert [m["step"] for m in published] == [1, 2, 3]
    last = published[-1]
    checkpoint.validate(last)

    # A fresh process: a different seed and a freshly initialised model, so
    # nothing carries over except what the checkpoint restores.
    resumed = train(
        epochs,
        checkpoint_cfg=cfg(mem_store, resume=last)[0],
        seed=999,
    )
    assert weights(resumed) == reference, (
        "resuming from the engine's own checkpoint must reproduce the "
        "uninterrupted run byte for byte"
    )


def test_every_checkpoint_the_loop_writes_satisfies_the_contract(mem_store):
    published = []
    train(
        4,
        checkpoint_cfg=cfg(mem_store)[0],
        emit=lambda p, pct, m=None: published.append(m["checkpoint"]) if p == "checkpoint" else None,
    )
    # After epochs 1..3; the last epoch is not checkpointed, a finished run's
    # checkpoint is the model.
    assert [m["step"] for m in published] == [1, 2, 3]
    for m in published:
        checkpoint.validate(m)
        assert m["resume_support"] == "bit_identical"
        assert m["code_hash"] == "sha256:code"
        assert m["rng_state"]["torch_cpu"]


def test_a_checkpoint_frame_does_not_rewind_the_progress_bar(mem_store):
    frames = []
    train(
        4,
        checkpoint_cfg=cfg(mem_store)[0],
        emit=lambda p, pct, m=None: frames.append((p, pct)),
    )
    ck = [pct for p, pct in frames if p == "checkpoint"]
    assert ck and all(pct > 0 for pct in ck), ck


# --- cadence ----------------------------------------------------------------


class FakeClock:
    def __init__(self):
        self.t = 0.0

    def __call__(self):
        return self.t


def tiny_model_and_opt():
    m = build()
    return m, torch.optim.Adam(m.parameters(), lr=1e-3)


def test_the_first_checkpoint_is_always_taken_to_measure_the_cost():
    store, clock = MemStore(), FakeClock()
    ck = checkpoint.TorchCheckpointer(
        store=store, key_prefix="p", code_hash="c", image_digest="i", dataset_id="d",
        mtbf_s=None, clock=clock,
    )
    assert ck.due()
    m, o = tiny_model_and_opt()
    assert ck.maybe_write(epoch=1, model=m, optimizer=o, dataloader_position=72) is not None
    assert ck.cost_s is not None and ck.cost_s > 0


def test_with_no_mtbf_there_is_no_interval_and_no_second_checkpoint():
    store, clock = MemStore(), FakeClock()
    ck = checkpoint.TorchCheckpointer(
        store=store, key_prefix="p", code_hash="c", image_digest="i", dataset_id="d",
        mtbf_s=None, clock=clock,
    )
    m, o = tiny_model_and_opt()
    ck.maybe_write(epoch=1, model=m, optimizer=o, dataloader_position=72)
    clock.t += 1e9
    assert ck.interval_s() is None
    assert not ck.due(), "an unknown failure rate gets one checkpoint, not a guess"


def test_the_next_checkpoint_is_due_at_the_young_daly_interval():
    store, clock = MemStore(), FakeClock()
    ck = checkpoint.TorchCheckpointer(
        store=store, key_prefix="p", code_hash="c", image_digest="i", dataset_id="d",
        mtbf_s=3600.0, clock=clock,
    )
    m, o = tiny_model_and_opt()
    ck.maybe_write(epoch=1, model=m, optimizer=o, dataloader_position=72)
    ck.cost_s = 10.0  # as if the write had taken ten seconds
    interval = ck.interval_s()
    assert interval == pytest.approx((2 * 10.0 * 3600.0) ** 0.5)
    clock.t = ck._last_end + interval - 1
    assert not ck.due()
    clock.t = ck._last_end + interval + 1
    assert ck.due()


def test_young_daly_matches_the_rust_side():
    assert checkpoint.young_daly_interval(10.0, 3600.0) == pytest.approx(268.328, abs=1e-3)
    assert checkpoint.young_daly_interval(0.0, 3600.0) is None
    assert checkpoint.young_daly_interval(10.0, 0.0) is None
    assert checkpoint.young_daly_interval(float("nan"), 3600.0) is None


# --- the request config -----------------------------------------------------


def test_checkpointing_is_on_only_for_frameworks_resume_was_measured_for():
    on = checkpoint.request_config(
        framework="torch", output_prefix="file:///a/b", dataset_hash="ds", resume=None
    )
    assert on is not None and on["key_prefix"] == "/a/b/checkpoints"
    assert on["dataset_id"] == "ds" and on["code_hash"].startswith("sha256:")
    for fw in ("sklearn", "garch", "xgboost", "lightgbm"):
        assert (
            checkpoint.request_config(
                framework=fw, output_prefix="p", dataset_hash="ds", resume=None
            )
            is None
        ), fw


def test_the_kill_switch_turns_it_off(monkeypatch):
    monkeypatch.setenv("TRAINER_CHECKPOINTS", "0")
    assert (
        checkpoint.request_config(
            framework="torch", output_prefix="p", dataset_hash="ds", resume=None
        )
        is None
    )


def test_a_checkpoint_whose_state_disagrees_with_its_manifest_is_corruption():
    store = MemStore()
    ck = checkpoint.TorchCheckpointer(
        store=store, key_prefix="p", code_hash="c", image_digest="i", dataset_id="d"
    )
    m, o = tiny_model_and_opt()
    manifest = ck.write(epoch=2, model=m, optimizer=o, dataloader_position=72)
    bad = dict(manifest, step=3)
    with pytest.raises(ValueError, match="manifest says 3"):
        checkpoint.restore_torch_checkpoint(store, bad, m, o)
