"""One training process, for `test_resume_producer.py` to spawn and kill.

This is not a test. It is the thing under test, run the way a worker runs it: a
fresh interpreter, the real `TrainRequest`, the real router, the real adapter,
the real artifact store. The suite spawns three of these and kills one.

Modes:

* `whole`    — train every epoch, write the bundle, exit 0.
* `partial`  — train with checkpointing, touch `--ready` once a checkpoint has
               reached the disk, then **block forever**. The parent kills it.
               Nothing here exits on its own; anything that did would weaken the
               test into "a clean shutdown left a file behind".
* `resume`   — continue from `--resume-from` at `--resume-step`, write the
               bundle, exit 0.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import pathlib
import sys
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))

import numpy as np  # noqa: E402
import pandas as pd  # noqa: E402

from app.artifact_store import get_store  # noqa: E402
from app.schemas import CheckpointConfig, ProgressConfig, TrainRequest  # noqa: E402
from app.worker import run_training  # noqa: E402

SEED = 20260915
N_ROWS = 320
CODE_HASH = "sha256:resume-producer-test"
IMAGE_DIGEST = "worker:trainer"
DATASET_ID = "snapshot:resume-producer-test"


def frame() -> pd.DataFrame:
    """The same rows in every process. Generated, not loaded, so there is no
    fixture for the three runs to drift from."""
    rng = np.random.default_rng(SEED)
    x = rng.normal(size=(N_ROWS, 5))
    coef = np.arange(1, 6, dtype=float)
    df = pd.DataFrame(x, columns=[f"f{i}" for i in range(5)])
    df["label"] = x @ coef + rng.normal(size=N_ROWS) * 0.25
    return df


def request(
    *, epochs: int, prefix: str, checkpoint: CheckpointConfig | None
) -> TrainRequest:
    return TrainRequest(
        run_id="resume-producer",
        model_id="m",
        model_kind="regressor",
        framework="torch",
        definition={
            "hyperparameters": {
                # `resolve_objective` and `resolve_seed` both read from here.
                "objective": "regression",
                "seed": SEED,
                "arch": "mlp",
                "epochs": epochs,
                "learning_rate": 1e-3,
                "batch_size": 64,
                "hidden_dim": 16,
            },
        },
        # Nothing at this URI: `_load_dataframe` falls back, so the driver
        # monkeypatches the loader below rather than materialising a Parquet.
        dataset_uri="file:///nowhere.parquet",
        dataset_hash="h",
        output_prefix=prefix,
        progress=ProgressConfig(nats_subject="resume.producer"),
        checkpoint=checkpoint,
    )


def train(req: TrainRequest):
    from app import worker as worker_mod

    worker_mod._load_dataframe = lambda _req: frame()  # noqa: SLF001
    return asyncio.run(run_training(req))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("whole", "partial", "resume"), required=True)
    parser.add_argument("--epochs", type=int, required=True)
    parser.add_argument("--kill-after", type=int, default=0)
    parser.add_argument("--ckpt-dir")
    parser.add_argument("--ready")
    parser.add_argument("--resume-from")
    parser.add_argument("--resume-step", type=int)
    parser.add_argument("--out")
    args = parser.parse_args(argv)

    if args.mode == "whole":
        resp = train(request(epochs=args.epochs, prefix="whole", checkpoint=None))
        if resp.status != "succeeded":
            print(resp.error, file=sys.stderr)
            return 1
        pathlib.Path(args.out).write_bytes(get_store().get(resp.artifact_uri))
        return 0

    if args.mode == "partial":
        # Interval 0: checkpoint at every epoch boundary. The cadence is the
        # caller's to choose (`CheckpointPolicy` has no default), and this caller
        # wants the earliest possible one so the kill has something to land after.
        config = CheckpointConfig(
            interval_seconds=0.0,
            code_hash=CODE_HASH,
            image_digest=IMAGE_DIGEST,
            dataset_id=DATASET_ID,
        )
        ckpt_dir = pathlib.Path(args.ckpt_dir)
        ckpt_dir.mkdir(parents=True, exist_ok=True)

        from app import worker as worker_mod

        original = worker_mod._build_checkpoint_policy  # noqa: SLF001

        def policy_that_announces(*a, **kw):
            built = original(*a, **kw)
            if built is None:
                return None
            written = a[4]

            def write(step: int, blob: bytes) -> None:
                built.write(step, blob)
                record = written[-1]
                (ckpt_dir / "manifest.json").write_text(json.dumps(record.manifest))
                (ckpt_dir / "blob.path").write_text(record.uri)
                if step >= args.kill_after:
                    # The checkpoint is on disk and referenced. From here the
                    # parent ends this process; there is no path out of this
                    # function that this process takes itself.
                    pathlib.Path(args.ready).write_text("ready")
                    while True:
                        time.sleep(0.05)

            return type(built)(
                interval_seconds=built.interval_seconds,
                write=write,
                resume_blob=built.resume_blob,
                resume_step=built.resume_step,
            )

        worker_mod._build_checkpoint_policy = policy_that_announces  # noqa: SLF001
        train(request(epochs=args.epochs, prefix="partial", checkpoint=config))
        # Only reachable if the run finished without ever reaching `kill_after`,
        # which means the experiment did not happen. Fail loudly.
        print(
            f"training finished without reaching epoch {args.kill_after}",
            file=sys.stderr,
        )
        return 2

    config = CheckpointConfig(
        interval_seconds=0.0,
        code_hash=CODE_HASH,
        image_digest=IMAGE_DIGEST,
        dataset_id=DATASET_ID,
        resume_from=args.resume_from,
        resume_step=args.resume_step,
    )
    resp = train(request(epochs=args.epochs, prefix="resumed", checkpoint=config))
    if resp.status != "succeeded":
        print(resp.error, file=sys.stderr)
        return 1
    if resp.resumed_from_step != args.resume_step:
        print(
            f"the response says it resumed from {resp.resumed_from_step}, not "
            f"{args.resume_step}",
            file=sys.stderr,
        )
        return 3
    if resp.metrics.get("resumed_from_epoch") != args.resume_step:
        print(
            f"the metrics say it resumed from {resp.metrics.get('resumed_from_epoch')}",
            file=sys.stderr,
        )
        return 4
    pathlib.Path(args.out).write_bytes(get_store().get(resp.artifact_uri))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
